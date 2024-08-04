//! # State of the system
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
    /// The blue button was pressed, the data is the number of presses
    BlueBtn(u32),
    /// The green button was pressed, the data is the number of presses
    GreenBtn(u32),
    /// The yellow button was pressed, the data is the number of presses
    YellowBtn(u32),
    /// The usb power state has changed, the data is the new state of the usb power
    Vbus(bool),
    /// The system power state has changed, the data is the new voltage of the system power
    Vsys(f32),
    /// The alarm settings have been read from the flash memory, the data is the alarm settings
    AlarmSettingsReadFromFlash(AlarmSettings),
    /// The alarm settings need to be updated in the flash memory
    AlarmSettingsNeedUpdate,
    /// The scheduler has ticked, the data is the time in (hour, minute, second)
    Scheduler((u8, u8, u8)),
    /// The rtc has been updated
    RtcUpdated,
    /// The system must go to standby mode
    Standby,
    /// The system must wake up from standby mode
    WakeUp,
    /// The alarm must be raised
    Alarm,
    /// The alarm must be stopped
    AlarmStop,
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
    /// Update the neopixel. The data is the time in (hour, minute, second), which will be displayed on the neopixel ring in the analog clock mode.
    /// Since the neopixel task runs on a different core, we cannot access the rtc there directly, unless we put it into a mutex, which is overkill
    /// for this simple task. So we will send the time to the neopixel task.
    /// We could theoretically put the time into the state of the system, but that would be a bit of a hack, since the time is not really part of the state of the system.
    /// Having two mutexes for the state of the system and the time would expose us to the risk of deadlocks, so all in all, it is better to send the time here.
    NeopixelUpdate((u8, u8, u8)),
    /// Update the sound task with the new state of the system
    /// ToDo: decide if and what data we need to send to the sound task
    SoundUpdate,
    /// Stop the minute timer
    MinuteTimerStop,
    /// Start the minute timer
    MinuteTimerStart,
}

/// For the events that we want the orchestrator to react to, all state events are of the type Enum Events.
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Events, 10> = Channel::new();

/// For the update commands that we want the orchestrator to send to the display task. Since we only ever want to display according to the state of
/// the system, we will not send any data in the command option and we can afford to work only with a simple state of "the display needs to be updated".
pub static DISPLAY_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// For the update commands that we want the orchestrator to send to the minute timer task. Since we only ever want to update the minute timer according to the state of
/// the system, we will not send any data in the command option and we can afford to work only with a simple state of "the minute timer needs to be stopped".
pub static TIMER_STOP_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();
pub static TIMER_START_SIGNAL: Signal<CriticalSectionRawMutex, Commands> = Signal::new();

/// Channel for the update commands that we want the orchestrator to send to the flash task.
pub static FLASH_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();

/// Channel for the update commands that we want the orchestrator to send to the neopixel.
pub static NEOPIXEL_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 3> = Channel::new();

/// Channel for the update commands that we want the orchestrator to send to the mp3-player task.
// pub static SOUND_CHANNEL: Channel<CriticalSectionRawMutex, Commands, 1> = Channel::new();

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
    pub alarm_settings: AlarmSettings,
    pub alarm_state: AlarmState,
    pub power_state: PowerState,
    // more
}

/// State transitions
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
                battery_voltage_fully_charged: 4.07,
                battery_voltage_empty: 2.6,
                battery_level: BatteryLevel::Bat000,
            },
        };
        manager
    }

    pub async fn toggle_alarm_enabled(&mut self) {
        self.alarm_settings.enabled = !self.alarm_settings.enabled;
        self.save_alarm_settings().await;
    }

    pub fn set_menu_mode(&mut self) {
        self.operation_mode = OperationMode::Menu;
    }

    pub fn set_normal_mode(&mut self) {
        self.operation_mode = OperationMode::Normal;
    }

    pub fn set_set_alarm_time_mode(&mut self) {
        self.operation_mode = OperationMode::SetAlarmTime;
    }

    pub fn set_alarm_mode(&mut self) {
        self.operation_mode = OperationMode::Alarm;
    }

    fn randomize_alarm_stop_buttom_sequence(&mut self) {}

    pub fn set_system_info_mode(&mut self) {
        self.operation_mode = OperationMode::SystemInfo;
    }

    pub fn increment_alarm_hour(&mut self) {
        let mut hour = self.alarm_settings.get_hour();
        hour = (hour + 1) % 24;
        self.alarm_settings
            .set_time((hour, self.alarm_settings.get_minute()));
    }

    pub fn increment_alarm_minute(&mut self) {
        let mut minute = self.alarm_settings.get_minute();
        minute = (minute + 1) % 60;
        self.alarm_settings
            .set_time((self.alarm_settings.get_hour(), minute));
    }

    pub async fn save_alarm_settings(&mut self) {
        let sender = EVENT_CHANNEL.sender();
        sender.send(Events::AlarmSettingsNeedUpdate).await;
    }

    pub async fn set_standby_mode(&mut self) {
        let sender = EVENT_CHANNEL.sender();
        self.operation_mode = OperationMode::Standby;
        sender.send(Events::Standby).await;
    }

    pub async fn wake_up(&mut self) {
        let sender = EVENT_CHANNEL.sender();
        self.set_normal_mode();
        sender.send(Events::WakeUp).await;
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
            OperationMode::Alarm => {}
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
            OperationMode::Alarm => {}
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
            OperationMode::Menu => {
                self.set_normal_mode();
            }
            OperationMode::SetAlarmTime => {
                self.increment_alarm_minute();
            }
            OperationMode::SystemInfo => {
                self.set_normal_mode();
            }
            OperationMode::Alarm => {}
            OperationMode::Standby => {
                self.wake_up().await;
            }
        }
    }
}

/// The operation mode of the system
#[derive(PartialEq, Debug, Format, Clone)]
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
#[derive(PartialEq, Debug, Format, Clone)]
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
#[derive(PartialEq, Debug, Format, Clone)]
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
    pub fn set_battery_level(&mut self) {
        if self.usb_power {
            self.battery_level = BatteryLevel::Charging;
        } else {
            // battery level is calculated based on the voltage of the battery, these are values measured on a LiPo battery on this system
            let upper_bound_voltage = self.battery_voltage_fully_charged;
            let lower_bound_voltage = self.battery_voltage_empty;

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

    pub fn get_battery_level(&self) -> BatteryLevel {
        self.battery_level.clone()
    }

    pub fn get_vsys(&self) -> f32 {
        self.vsys
    }

    pub fn get_usb_power(&self) -> bool {
        self.usb_power
    }

    pub fn get_battery_voltage_fully_charged(&self) -> f32 {
        self.battery_voltage_fully_charged
    }

    pub fn get_battery_voltage_empty(&self) -> f32 {
        self.battery_voltage_empty
    }

    pub fn set_vsys(&mut self, vsys: f32) {
        self.vsys = vsys;
    }

    pub fn set_usb_power(&mut self, usb_power: bool) {
        self.usb_power = usb_power;
    }
}
