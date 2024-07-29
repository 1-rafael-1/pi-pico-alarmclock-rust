//! This module desccribes the state of the system and the events that can change the state of the system as well as the commands that can be sent to the tasks
//! that control the system.
use defmt::*;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;

/// Events that we want to react to together with the data that we need to react to the event.
/// Works in conjunction with the `EVENT_CHANNEL` channel in the orchestrator task.
#[derive(PartialEq, Debug, Format)]
pub enum Events {
    BlueBtn(u32),
    GreenBtn(u32),
    YellowBtn(u32),
    Vbus(bool),
    Vsys(f32),
    AlarmSettingsReadFromFlash(AlarmSettings),
    MinuteTimer,
}

/// Commands that we want to send from the orchestrator to the other tasks that we want to control.
/// Works in conjunction with the `COMMAND_CHANNEL` channel in the orchestrator task.
#[derive(PartialEq, Debug, Format)]
pub enum Commands {
    /// Write the alarm settings to the flash memory, the data is the alarm settings
    /// Since the alarm settings are small amd rarely changed, we can send them in the command option
    AlarmSettingsWriteToFlash(AlarmSettings),
    /// Update the display with the new state of the system
    /// Since we will need to update the display often and wizth a lot of data, we will not send the data in the command option
    DisplayUpdate,
    /// Update the neopixel with the new state of the system
    /// ToDo: decide if and what data we need to send to the neopixel
    NeopixelUpdate,
    /// Update the sound task with the new state of the system
    /// ToDo: decide if and what data we need to send to the sound task
    SoundUpdate,
}

/// For the events that we want the orchestrator to react to, all state events are of the type Enum Events.
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Events, 10> = Channel::new();

/// For the update commands that we want the orchestrator to send to the display task. Since we only ever want to display according to the state of
/// the system, we will not send any data in the command option and we can afford to work only with a simple state of "the display needs to be updated".
pub static DISPLAY_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// Channel for the update commands that we want the orchestrator to send to the neopixel.
pub static NEOPIXEL_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 3> = Channel::new();

/// Channel for the update commands that we want the orchestrator to send to the flash task.
pub static FLASH_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();

/// Channel for the update commands that we want the orchestrator to send to the mp3-player task.
pub static SOUND_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();

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
#[derive(PartialEq, Debug, Format)]
pub struct StateManager {
    /// The operation mode of the system
    pub operation_mode: OperationMode,
    pub alarm_settings: AlarmSettings,
    pub alarm_state: AlarmState,
    pub power_state: PowerState,
    // more
}

impl StateManager {
    /// Create a new StateManager.             
    /// We will get the actual data pretty early in the system startup, so we can set all this to inits here
    pub fn new() -> Self {
        let manager = StateManager {
            operation_mode: OperationMode::Normal,
            alarm_settings: AlarmSettings::new_empty(),
            alarm_state: AlarmState::None,
            power_state: PowerState {
                usb_power: false,
                vsys: 0.0,
                battery_level: BatteryLevel::Bat000,
            },
        };
        manager
    }

    fn toggle_alarm_enabled(&mut self) {
        self.alarm_settings.enabled = !self.alarm_settings.enabled;
    }

    /// Handle presses of the green button
    pub fn handle_green_button_press(&mut self) {
        match self.operation_mode {
            OperationMode::Normal => {
                self.toggle_alarm_enabled();
            }
            _ => {
                // ToDo: handle the green button press in other operation modes
            }
        }
    }

    // ToDo: handle the other button presses
}

/// The operation mode of the system
#[derive(PartialEq, Debug, Format)]
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
}

/// The settings for the alarm
#[derive(PartialEq, Debug, Format, Clone)]
pub struct AlarmSettings {
    /// The alarm time is set to the specified time
    time: (u8, u8),
    /// The alarm is enabled or disabled
    enabled: bool,
}

impl AlarmSettings {
    pub fn new_empty() -> Self {
        AlarmSettings {
            time: (0, 0),
            enabled: false,
        }
    }

    pub fn set_time(&mut self, time: (u8, u8)) {
        self.time = time;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn get_hour(&self) -> u8 {
        self.time.0
    }

    pub fn get_minute(&self) -> u8 {
        self.time.1
    }

    pub fn get_enabled(&self) -> bool {
        self.enabled
    }
}

/// The state of the alarm
#[derive(PartialEq, Debug, Format)]
pub enum AlarmState {
    /// The alarm is not active, the alarm time has not been reached
    None,
    /// The alarm time has been reached, the alarm is active and the sunrise effect is displayed on the neopixel ring. The user
    /// can stop the alarm by pressing the buttons in the correct sequence.
    Sunrise,
    /// We are past the sunrise effect. The alarm sound is playing, the neopixel waker effect is playing. The user can stop the alarm by pressing
    /// the buttons in the correct sequence.
    Noise,
    /// The alarm is being stopped after the correct button sequence has been pressed. The next state will be None.
    StopAlarm,
}

/// The battery level of the system in steps of 20% from 0 to 100. One additional state is provided for charging.
#[derive(PartialEq, Debug, Format)]
pub enum BatteryLevel {
    Charging,
    Bat000,
    Bat020,
    Bat040,
    Bat060,
    Bat080,
    Bat100,
}

/// The power state of the system
#[derive(PartialEq, Debug, Format)]
pub struct PowerState {
    /// The system is running on usb power
    pub usb_power: bool,
    /// The voltage of the system power supply
    pub vsys: f32,
    /// The battery level of the system
    /// The battery level is provided in steps of 20% from 0 to 100. One additional state is provided for charging.
    battery_level: BatteryLevel,
}

impl PowerState {
    pub fn set_battery_level(&mut self) {
        if self.usb_power {
            self.battery_level = BatteryLevel::Charging;
        } else {
            // battery level is calculated based on the voltage of the battery, these are values measured on a LiPo battery on this system
            let upper_bound_voltage = 4.1; // fully charged battery
            let lower_bound_voltage = 2.6; // empty battery

            // Calculate battery level based on voltage
            let battery_percent = ((self.vsys - lower_bound_voltage)
                / (upper_bound_voltage - lower_bound_voltage)
                * 100.0) as u8;
            // set the battery level
            self.battery_level = match battery_percent {
                0..=5 => BatteryLevel::Bat000,
                6..=29 => BatteryLevel::Bat020,
                30..=49 => BatteryLevel::Bat040,
                50..=69 => BatteryLevel::Bat060,
                70..=89 => BatteryLevel::Bat080,
                _ => BatteryLevel::Bat100,
            };
        }
    }
}

/// The menu mode of the system
#[derive(PartialEq, Debug, Format)]
pub enum MenuMode {
    /// The default state: the clock is displayed
    None,
    /// The system info menu is being displayed
    SystemInfoMenu,
}

/// options for the system info
#[derive(PartialEq, Debug, Format)]
pub enum SystemInfoMenuMode {
    /// select to either display the system info or shutdown the system
    Select,
    /// display the system info
    Info,
    /// shutdown the system into a low power state
    ShutdownLowPower,
}
