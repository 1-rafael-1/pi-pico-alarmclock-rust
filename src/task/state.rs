//! # State of the system
//! This module desccribes the state of the system and the operations that can be performed on the state.
use crate::task::buttons::Button;
use crate::task::task_messages::{EVENT_CHANNEL, Events};
use defmt::Format;
use embassy_rp::clocks::RoscRng;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use rand::Rng;

/// Type alias for the system state manager protected by a mutex.
///
/// This type alias defines a `Mutex` that uses a `CriticalSectionRawMutex` for synchronization.
/// The state is wrapped in an `Option` to allow for the possibility of the state being uninitialized.
/// This ensures that tasks can safely access and update the state across different executors (e.g., different cores).
type StateManagerType = Mutex<CriticalSectionRawMutex, Option<StateManager>>;

/// Global instance of the system state manager protected by a mutex.
///
/// This static variable holds the system state manager, which is protected by a `Mutex` to ensure
/// that only one task can access the state at a time. The mutex uses a `CriticalSectionRawMutex`
/// for synchronization, allowing safe access across different tasks and executors.
///
/// The state is initially set to `None`, indicating that it has not been initialized yet.
/// Tasks attempting to access the state before initialization will need to handle the `None` case.
pub static STATE_MANAGER_MUTEX: StateManagerType = Mutex::new(None);

/// All the states of the system are kept in this struct.
#[derive(PartialEq, Debug, Format, Clone)]
pub struct StateManager {
    /// The operation mode of the system
    pub operation_mode: OperationMode,
    /// The settings for the alarm
    pub alarm_settings: AlarmSettings,
    /// The state of the alarm
    pub alarm_state: AlarmState,
    /// The power state of the system
    pub power_state: PowerState,
}

/// State transitions
impl StateManager {
    /// Create a new `StateManager`.
    /// We will get the actual data pretty early in the system startup, so we can set all this to inits here
    pub fn new() -> Self {
        Self {
            operation_mode: OperationMode::Normal,
            alarm_settings: AlarmSettings::new_empty(),
            alarm_state: AlarmState::None,
            power_state: PowerState {
                usb_power: false,
                vsys: 0.0,
                battery_voltage_fully_charged: 4.07,
                battery_voltage_empty: 2.6,
                battery_level: BatteryLevel::Bat000,
            },
        }
    }

    /// Toggle the alarm enabled state
    pub async fn toggle_alarm_enabled(&mut self) {
        self.alarm_settings
            .set_enabled(!self.alarm_settings.get_enabled());
        self.save_alarm_settings().await;
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
    }

    /// Increment the alarm hour
    pub fn increment_alarm_hour(&mut self) {
        self.alarm_settings.increment_alarm_hour();
    }

    /// Increment the alarm minute
    pub fn increment_alarm_minute(&mut self) {
        self.alarm_settings.increment_alarm_minute();
    }

    /// Save the alarm settings
    pub async fn save_alarm_settings(&self) {
        let sender = EVENT_CHANNEL.sender();
        sender.send(Events::AlarmSettingsNeedUpdate).await;
    }

    /// Set the system to standby mode
    pub async fn set_standby_mode(&mut self) {
        let sender = EVENT_CHANNEL.sender();
        self.operation_mode = OperationMode::Standby;
        sender.send(Events::Standby).await;
    }

    /// Wake up the system from standby mode
    pub async fn wake_up(&mut self) {
        let sender = EVENT_CHANNEL.sender();
        self.set_normal_mode();
        sender.send(Events::WakeUp).await;
    }

    /// Randomize the alarm stop button sequence
    pub fn randomize_alarm_stop_buttom_sequence(&mut self) {
        self.alarm_settings.randomize_stop_alarm_button_sequence();
    }
}

/// User Input Handling
impl StateManager {
    /// Handle state changes when the green button is pressed
    pub async fn handle_green_button_press(&mut self) {
        match self.operation_mode {
            OperationMode::Normal => {
                self.toggle_alarm_enabled().await;
            }
            OperationMode::SetAlarmTime => {
                self.increment_alarm_hour();
            }
            OperationMode::Menu => {
                self.set_system_info_mode();
            }
            OperationMode::SystemInfo => {
                self.set_normal_mode();
            }
            OperationMode::Alarm => {
                if self.alarm_settings.get_first_valid_stop_alarm_button() == Button::Green {
                    self.alarm_settings.erase_first_valid_stop_alarm_button();
                };
                if self.alarm_settings.is_alarm_stop_button_sequence_complete() {
                    EVENT_CHANNEL.sender().send(Events::AlarmStop).await;
                }
            }
            OperationMode::Standby => {
                self.wake_up().await;
            }
        }
    }

    /// Handle state changes when the blue button is pressed
    pub async fn handle_blue_button_press(&mut self) {
        match self.operation_mode {
            OperationMode::Normal => {
                self.set_set_alarm_time_mode();
            }
            OperationMode::SetAlarmTime => {
                self.save_alarm_settings().await;
                self.set_normal_mode();
            }
            OperationMode::Menu => {
                self.set_standby_mode().await;
            }
            OperationMode::SystemInfo => {
                self.set_normal_mode();
            }
            OperationMode::Alarm => {
                if self.alarm_settings.get_first_valid_stop_alarm_button() == Button::Blue {
                    self.alarm_settings.erase_first_valid_stop_alarm_button();
                };
                if self.alarm_settings.is_alarm_stop_button_sequence_complete() {
                    EVENT_CHANNEL.sender().send(Events::AlarmStop).await;
                }
            }
            OperationMode::Standby => {
                self.wake_up().await;
            }
        }
    }

    /// Handle state changes when the yellow button is pressed
    pub async fn handle_yellow_button_press(&mut self) {
        match self.operation_mode {
            OperationMode::Normal => {
                self.set_menu_mode();
            }
            OperationMode::Menu | OperationMode::SystemInfo => {
                self.set_normal_mode();
            }
            OperationMode::SetAlarmTime => {
                self.increment_alarm_minute();
            }
            OperationMode::Alarm => {
                if self.alarm_settings.get_first_valid_stop_alarm_button() == Button::Yellow {
                    self.alarm_settings.erase_first_valid_stop_alarm_button();
                };
                if self.alarm_settings.is_alarm_stop_button_sequence_complete() {
                    EVENT_CHANNEL.sender().send(Events::AlarmStop).await;
                }
            }
            OperationMode::Standby => {
                self.wake_up().await;
            }
        }
    }
}

/// The operation mode of the system
#[derive(Eq, PartialEq, Debug, Format, Clone)]
pub enum OperationMode {
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
    /// The system is in standby mode, the display is off, the neopixel ring is off, the system is in a low power state.
    Standby,
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

    /// The sequence gets iterated and the first of its values that is None is returned.
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
            let battery_percent = (self.vsys - lower_bound_voltage)
                / (upper_bound_voltage - lower_bound_voltage)
                * 100.0;
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

    /// Get the battery voltage when fully charged
    pub const fn get_battery_voltage_fully_charged(&self) -> f32 {
        self.battery_voltage_fully_charged
    }

    /// Get the battery voltage when empty
    pub const fn get_battery_voltage_empty(&self) -> f32 {
        self.battery_voltage_empty
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
