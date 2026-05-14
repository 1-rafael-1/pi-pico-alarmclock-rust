//! # System State
//! This module describes the state of the system and the operations that can be performed on the state.
use defmt::Format;
use embassy_rp::clocks::RoscRng;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use rand::Rng;

use crate::task::buttons::Button;

/// Type alias for the system state protected by a mutex.
///
/// This type alias defines a `Mutex` that uses a `CriticalSectionRawMutex` for synchronization.
/// The state is wrapped in an `Option` to allow for the possibility of the state being uninitialized.
/// This ensures that tasks can safely access and update the state across different executors (e.g., different cores).
type SystemStateType = Mutex<CriticalSectionRawMutex, Option<SystemState>>;

/// Global instance of the system state protected by a mutex.
///
/// This static variable holds the system state, which is protected by a `Mutex` to ensure
/// that only one task can access the state at a time. The mutex uses a `CriticalSectionRawMutex`
/// for synchronization, allowing safe access across different tasks and executors.
///
/// The state is initially set to `None`, indicating that it has not been initialized yet.
/// Tasks attempting to access the state before initialization will need to handle the `None` case.
pub static SYSTEM_STATE: SystemStateType = Mutex::new(None);

/// All the states of the system are kept in this struct.
#[derive(PartialEq, Debug, Format, Clone)]
pub struct SystemState {
    /// The operation mode of the system
    pub operation_mode: OperationMode,
    /// The settings for the alarm
    pub alarm_settings: AlarmSettings,
    /// User-configurable system settings (volume, brightness, etc.)
    pub system_settings: SystemSettings,
    /// The state of the alarm
    pub alarm_state: AlarmState,
    /// The power state of the system
    pub power_state: PowerState,
    /// The current page of system info (0-based, 0 = power info, 1 = alarm info)
    pub system_info_page: u8,
    /// Temporary buffer for manual time setting (hour, minute)
    pub manual_time_buffer: (u8, u8),
    /// The tick count of the last user interaction (for menu timeout)
    pub last_interaction_tick: u64,
    /// Whether the display is currently on or off
    pub display_state: DisplayState,
}

/// State transitions and operations
impl SystemState {
    /// Create a new `SystemState`.
    /// We will get the actual data pretty early in the system startup, so we can set all this to inits here
    pub const fn new() -> Self {
        Self {
            operation_mode: OperationMode::Initializing,
            alarm_settings: AlarmSettings::new_empty(),
            system_settings: SystemSettings::new_default(),
            alarm_state: AlarmState::None,
            power_state: PowerState {
                usb_power: false,
                vsys: 0.0,
                battery_voltage_fully_charged: 4.07,
                battery_voltage_empty: 2.6,
                battery_level: BatteryLevel::Bat000,
            },
            system_info_page: 0,
            manual_time_buffer: (0, 0),
            last_interaction_tick: 0,
            display_state: DisplayState::On,
        }
    }

    /// Toggle the alarm enabled state
    pub const fn toggle_alarm_enabled(&mut self) {
        self.alarm_settings.set_enabled(!self.alarm_settings.get_enabled());
    }

    /// Set the system to menu mode
    pub const fn set_menu_mode(&mut self) {
        self.operation_mode = OperationMode::Menu;
    }

    /// Set the system to normal mode
    pub const fn set_normal_mode(&mut self) {
        self.operation_mode = OperationMode::Normal;
        self.set_alarm_state(AlarmState::None);
    }

    /// Transition from initializing to normal mode (only allowed from Initializing state)
    pub const fn complete_initialization(&mut self) {
        if matches!(self.operation_mode, OperationMode::Initializing) {
            self.operation_mode = OperationMode::Normal;
        }
    }

    /// Set the system to set alarm time mode
    pub const fn set_set_alarm_time_mode(&mut self) {
        self.operation_mode = OperationMode::SetAlarmTime;
    }

    /// Set the system to alarm mode
    pub const fn set_alarm_mode(&mut self) {
        self.operation_mode = OperationMode::Alarm;
        self.set_alarm_state(AlarmState::Sunrise);
    }

    /// Set the alarm state
    pub const fn set_alarm_state(&mut self, state: AlarmState) {
        self.alarm_state = state;
    }

    /// Set the system to system info mode
    pub const fn set_system_info_mode(&mut self) {
        self.operation_mode = OperationMode::SystemInfo;
        self.system_info_page = 0;
    }

    /// Set the system to settings menu mode
    pub const fn set_settings_menu_mode(&mut self) {
        self.operation_mode = OperationMode::SettingsMenu;
    }

    /// Set the system to set volume mode
    pub const fn set_volume_mode(&mut self) {
        self.operation_mode = OperationMode::SetVolume;
    }

    /// Set the system to set clock brightness mode
    pub const fn set_clock_brightness_mode(&mut self) {
        self.operation_mode = OperationMode::SetClockBrightness;
    }

    /// Set the system to manual time setting mode, initializing buffer with current RTC time
    pub const fn set_time_manual_mode(&mut self, current_hour: u8, current_minute: u8) {
        self.manual_time_buffer = (current_hour, current_minute);
        self.operation_mode = OperationMode::SetTimeManual;
    }

    /// Increment the manual time hour
    pub const fn increment_manual_hour(&mut self) {
        self.manual_time_buffer.0 = (self.manual_time_buffer.0 + 1) % 24;
    }

    /// Increment the manual time minute
    pub const fn increment_manual_minute(&mut self) {
        self.manual_time_buffer.1 = (self.manual_time_buffer.1 + 1) % 60;
    }

    /// Advance to the next system info page, or exit if on the last page
    pub const fn next_system_info_page(&mut self) -> bool {
        if self.system_info_page == 0 {
            self.system_info_page = 1;
            true // Still in system info mode
        } else {
            false // Should exit system info mode
        }
    }

    /// Update the last interaction tick to current time
    pub const fn update_interaction_tick(&mut self, tick: u64) {
        self.last_interaction_tick = tick;
    }

    /// Turn the display on
    pub const fn set_display_on(&mut self) {
        self.display_state = DisplayState::On;
    }

    /// Turn the display off
    pub const fn set_display_off(&mut self) {
        self.display_state = DisplayState::Off;
    }

    /// Check if the display is off
    pub fn is_display_off(&self) -> bool {
        self.display_state == DisplayState::Off
    }

    /// Check if the display should sleep when the alarm is enabled
    pub fn should_alarm_display_sleep(&self, current_tick: u64) -> bool {
        const DISPLAY_SLEEP_TIMEOUT_MS: u64 = 30_000;

        self.alarm_settings.get_enabled()
            && matches!(self.operation_mode, OperationMode::Normal)
            && self.display_state == DisplayState::On
            && (current_tick.saturating_sub(self.last_interaction_tick) >= DISPLAY_SLEEP_TIMEOUT_MS)
    }

    /// Check if any interactive mode should timeout (10 seconds = 10000ms of inactivity)
    pub const fn should_interactive_mode_timeout(&self, current_tick: u64) -> bool {
        matches!(
            self.operation_mode,
            OperationMode::Menu
                | OperationMode::SettingsMenu
                | OperationMode::SystemInfo
                | OperationMode::SetAlarmTime
                | OperationMode::SetVolume
                | OperationMode::SetClockBrightness
                | OperationMode::SetTimeManual
        ) && (current_tick.saturating_sub(self.last_interaction_tick) >= 10000)
    }

    /// Increment the alarm hour
    pub const fn increment_alarm_hour(&mut self) {
        self.alarm_settings.increment_alarm_hour();
    }

    /// Increment the alarm minute
    pub const fn increment_alarm_minute(&mut self) {
        self.alarm_settings.increment_alarm_minute();
    }

    /// Set the system to standby mode
    pub const fn set_standby_mode(&mut self) {
        self.operation_mode = OperationMode::Standby;
    }

    /// Wake up the system from standby mode
    pub const fn wake_up(&mut self) {
        self.set_normal_mode();
    }

    /// Randomize the alarm stop button sequence
    pub fn randomize_alarm_stop_button_sequence(&mut self) {
        self.alarm_settings.randomize_stop_alarm_button_sequence();
    }
}

/// The operation mode of the system
#[derive(Eq, PartialEq, Debug, Format, Clone)]
pub enum OperationMode {
    /// The system is initializing, waiting for RTC and alarm settings to be loaded.
    ///
    /// Displays the setup icon and "Initializing" text. Does not update the neopixel ring.
    Initializing,
    /// The regular operation mode.
    ///
    /// Displays the time, the alarm status, etc. Showing the analog clock on the neopixel
    /// ring, if the alarm is active.
    Normal,
    /// Setting the alarm time.
    ///
    /// Displays the alarm time and allowing the user to set the new alarm time.
    SetAlarmTime,
    /// The alarm is active, starting with the sunrise effect on the neopixel ring, then playing the alarm sound and displaying the waker effect on the neopixel ring.
    /// on the neopixel ring. Also display and await the color sequence of buttons that need to be pressed to stop the alarm.
    Alarm,
    /// The menu is active, displaying the menu options and allowing the user to select the menu options.
    Menu,
    /// Displaying the system info
    SystemInfo,
    /// The settings menu is active, displaying settings options
    SettingsMenu,
    /// Setting the volume level
    SetVolume,
    /// Setting the clock brightness level
    SetClockBrightness,
    /// Setting the time manually
    SetTimeManual,
    /// The system is in standby mode, the display is off, the neopixel ring is off, the system is in a low power state.
    Standby,
}

/// The on/off state of the display
#[derive(Eq, PartialEq, Debug, Format, Clone)]
pub enum DisplayState {
    /// Display is on
    On,
    /// Display is off
    Off,
}

/// User-configurable system settings (persisted to flash)
#[derive(Eq, PartialEq, Debug, Format, Clone, Copy)]
pub struct SystemSettings {
    /// Volume level for alarm sound (0-30, `DFPlayer` range)
    volume: u8,
    /// Brightness for `NeoPixel` clock mode (0-20, reasonable range to avoid power issues)
    clock_brightness: u8,
}

impl SystemSettings {
    /// Create new settings with sensible defaults
    pub const fn new_default() -> Self {
        Self {
            volume: 13,          // Current hardcoded value
            clock_brightness: 1, // Current hardcoded value
        }
    }

    /// Get the volume level
    pub const fn get_volume(self) -> u8 {
        self.volume
    }

    /// Set the volume level (clamped to valid `DFPlayer` range)
    pub const fn set_volume(&mut self, volume: u8) {
        self.volume = if volume > 30 { 30 } else { volume };
    }

    /// Increment the volume level (wraps from 30 to 0)
    pub const fn increment_volume(&mut self) {
        self.volume = if self.volume >= 30 { 0 } else { self.volume + 1 };
    }

    /// Decrement the volume level (wraps from 0 to 30)
    pub const fn decrement_volume(&mut self) {
        self.volume = if self.volume == 0 { 30 } else { self.volume - 1 };
    }

    /// Get the clock brightness level
    pub const fn get_clock_brightness(self) -> u8 {
        self.clock_brightness
    }

    /// Set the clock brightness level (clamped to reasonable range)
    pub const fn set_clock_brightness(&mut self, brightness: u8) {
        self.clock_brightness = if brightness > 20 { 20 } else { brightness };
    }

    /// Increment the clock brightness level (wraps from 20 to 0)
    pub const fn increment_clock_brightness(&mut self) {
        self.clock_brightness = if self.clock_brightness >= 20 {
            0
        } else {
            self.clock_brightness + 1
        };
    }

    /// Decrement the clock brightness level (wraps from 0 to 20)
    pub const fn decrement_clock_brightness(&mut self) {
        self.clock_brightness = if self.clock_brightness == 0 {
            20
        } else {
            self.clock_brightness - 1
        };
    }
}

/// The settings for the alarm
#[derive(Eq, PartialEq, Debug, Format, Clone)]
pub struct AlarmSettings {
    /// The alarm time is set to the specified time
    time: (u8, u8),
    /// The alarm is enabled or disabled
    enabled: bool,
    /// The color sequence of buttons that need to be pressed to stop the alarm
    stop_alarm_button_sequence: [Button; 3],
}

impl AlarmSettings {
    /// Create a new `AlarmSettings` with default values.
    pub const fn new_empty() -> Self {
        Self {
            time: (0, 0),
            enabled: false,
            stop_alarm_button_sequence: [Button::Green, Button::Blue, Button::Yellow],
        }
    }

    /// Set the alarm time
    pub const fn set_time(&mut self, time: (u8, u8)) {
        self.time = time;
    }

    /// Set the enabled state
    pub const fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Get the alarm time hour
    pub const fn get_hour(&self) -> u8 {
        self.time.0
    }

    /// Get the alarm time minute
    pub const fn get_minute(&self) -> u8 {
        self.time.1
    }

    /// Get the enabled state
    pub const fn get_enabled(&self) -> bool {
        self.enabled
    }

    /// Increment the alarm hour
    pub const fn increment_alarm_hour(&mut self) {
        let mut hour = self.get_hour();
        hour = (hour + 1) % 24;
        self.set_time((hour, self.get_minute()));
    }

    /// Increment the alarm minute
    pub const fn increment_alarm_minute(&mut self) {
        let mut minute = self.get_minute();
        minute = (minute + 1) % 60;
        self.set_time((self.get_hour(), minute));
    }

    /// Get the stop alarm button sequence
    pub fn get_stop_alarm_button_sequence(&self) -> [Button; 3] {
        self.stop_alarm_button_sequence.clone()
    }

    /// Set the stop alarm button sequence
    const fn set_stop_alarm_button_sequence(&mut self, sequence: [Button; 3]) {
        self.stop_alarm_button_sequence = sequence;
    }

    /// Randomize the stop alarm button sequence. In no-std, we have limited options for random number generation and there is no shuffle method.
    /// So we will use a Fisher-Yates shuffle algorithm likeness to shuffle the sequence.
    pub fn randomize_stop_alarm_button_sequence(&mut self) {
        let mut sequence = [Button::Green, Button::Blue, Button::Yellow];
        for i in 0..sequence.len() {
            let j = RoscRng.gen_range(0..sequence.len());
            sequence.swap(i, j);
        }
        self.set_stop_alarm_button_sequence(sequence);
    }

    /// The sequence gets iterated and the first of its values that is not None is set to None.
    pub fn erase_first_valid_stop_alarm_button(&mut self) {
        let mut sequence = self.get_stop_alarm_button_sequence();
        let mut i = 0;
        while i < sequence.len() && sequence[i] == Button::None {
            i += 1;
        }
        if i < sequence.len() {
            sequence[i] = Button::None;
        }
        self.set_stop_alarm_button_sequence(sequence);
    }

    /// The sequence gets iterated and the first of its values that is not None is returned.
    pub fn get_first_valid_stop_alarm_button(&self) -> Button {
        let sequence = self.get_stop_alarm_button_sequence();
        let mut i = 0;
        while i < sequence.len() && sequence[i] == Button::None {
            i += 1;
        }
        if i < sequence.len() {
            sequence[i].clone()
        } else {
            Button::None
        }
    }

    /// Check if the alarm stop button sequence is complete
    pub fn is_alarm_stop_button_sequence_complete(&self) -> bool {
        let sequence = self.get_stop_alarm_button_sequence();
        // Check if all buttons in the sequence are None
        sequence.iter().all(|button| *button == Button::None)
    }
}

/// The state of the alarm
#[derive(Eq, PartialEq, Debug, Format, Clone)]
pub enum AlarmState {
    /// The alarm is not active
    None,
    /// The alarm time has been reached, the alarm is active and the sunrise effect is displayed on the neopixel ring. The user
    /// can stop the alarm by pressing the buttons in the correct sequence.
    Sunrise,
    /// We are past the sunrise effect. The alarm sound is playing, the neopixel waker effect is playing. The user can stop the alarm by pressing
    /// the buttons in the correct sequence.
    Noise,
}

impl AlarmState {
    /// Check if the alarm is active
    pub fn is_active(&self) -> bool {
        self != &Self::None
    }
}

/// The battery level of the system in steps of 20% from 0 to 100. One additional state is provided for charging.
#[derive(Eq, PartialEq, Debug, Format, Clone)]
pub enum BatteryLevel {
    /// The battery is charging
    Charging,
    /// The battery level is 0%
    Bat000,
    /// The battery level is 20%
    Bat020,
    /// The battery level is 40%
    Bat040,
    /// The battery level is 60%
    Bat060,
    /// The battery level is 80%
    Bat080,
    /// The battery level is 100%
    Bat100,
}

/// The power state of the system
#[derive(PartialEq, Debug, Format, Clone)]
pub struct PowerState {
    /// The system is running on usb power
    usb_power: bool,
    /// The voltage of the system power supply
    vsys: f32,
    /// The battery voltage when fully charged
    battery_voltage_fully_charged: f32,
    /// The battery voltage when the charger board cuts off the battery
    battery_voltage_empty: f32,
    /// The battery level of the system
    /// The battery level is provided in steps of 20% from 0 to 100. One additional state is provided for charging.
    battery_level: BatteryLevel,
}

impl PowerState {
    /// Set the battery level based on the current vsys voltage and usb power state
    pub fn set_battery_level(&mut self) {
        if self.usb_power {
            self.battery_level = BatteryLevel::Charging;
        } else {
            // battery level is calculated based on the voltage of the battery, these are values measured on a LiPo battery on this system
            let upper_bound_voltage = self.battery_voltage_fully_charged;
            let lower_bound_voltage = self.battery_voltage_empty;

            // Calculate battery level based on voltage
            let battery_percent =
                (self.vsys - lower_bound_voltage) / (upper_bound_voltage - lower_bound_voltage) * 100.0;
            // set the battery level
            self.battery_level = match battery_percent {
                0f32..=5f32 => BatteryLevel::Bat000,
                6f32..=29f32 => BatteryLevel::Bat020,
                30f32..=49f32 => BatteryLevel::Bat040,
                50f32..=69f32 => BatteryLevel::Bat060,
                70f32..=89f32 => BatteryLevel::Bat080,
                _ => BatteryLevel::Bat100,
            };
        }
    }

    /// Get the battery level
    pub fn get_battery_level(&self) -> BatteryLevel {
        self.battery_level.clone()
    }

    /// Get the vsys voltage
    pub const fn get_vsys(&self) -> f32 {
        self.vsys
    }

    /// Get the usb power state
    pub const fn get_usb_power(&self) -> bool {
        self.usb_power
    }

    /// Set the vsys voltage
    pub const fn set_vsys(&mut self, vsys: f32) {
        self.vsys = vsys;
    }

    /// Set the usb power state
    pub fn set_usb_power(&mut self, usb_power: bool) {
        self.usb_power = usb_power;
        self.set_battery_level();
    }
}
