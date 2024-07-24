//! # State
//! This module keeps the state of the system.
//! This module is responsible for the state transitions of the system, receiving events from the various tasks and reacting to them.
//! Reacting to the events will involve changing the state of the system and triggering actions like updating the display, playing sounds, etc.
use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::Rtc;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

/// Task configuration
/// This struct is used to configure which tasks are enabled
/// This is useful for troubleshooting, as we can disable tasks to reduce the binary size
/// and clutter in the output.
/// Also, we can disable tasks that are not needed for the current development stage and also test tasks in isolation.
/// For a production build we will need all tasks enabled
pub struct TaskConfig {
    pub btn_green: bool,
    pub btn_blue: bool,
    pub btn_yellow: bool,
    pub time_updater: bool,
    pub neopixel: bool,
    pub display: bool,
    pub dfplayer: bool,
    pub usb_power: bool,
    pub vsys_voltage: bool,
}

impl Default for TaskConfig {
    fn default() -> Self {
        TaskConfig {
            btn_green: true,
            btn_blue: true,
            btn_yellow: true,
            time_updater: true,
            neopixel: true,
            display: true,
            dfplayer: true,
            usb_power: true,
            vsys_voltage: true,
        }
    }
}

impl TaskConfig {
    pub fn new() -> Self {
        TaskConfig::default()
    }
}

/// Events that we want to react to together with the data that we need to react to the event
#[derive(PartialEq, Debug, Format)]
pub enum Events {
    BlueBtn(u32),
    GreenBtn(u32),
    YellowBtn(u32),
    Vbus(bool),
    Vsys(f32),
    // more
}

/// Channel for the events that we want to react to, all state events are of the type Enum Events
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Events, 10> = Channel::new();

#[derive(PartialEq, Debug, Format)]
pub struct StateManager {
    pub state: State,
    pub menu: Menu,
    pub system_info: SystemInfo,
    pub alarm_time: (u8, u8),
    pub alarm_enabled: bool,
    pub power_state: PowerState,
    // more
}

/// global state
#[derive(PartialEq, Debug, Format)]
pub enum State {
    Idle,
    Menu,
    Alarm,
    SystemInfo,
}

impl State {
    pub fn toggle_alarm_active(&mut self) {
        match self {
            State::Alarm => {
                *self = State::Idle;
            }
            _ => {
                *self = State::Alarm;
            }
        }
    }
}

/// options for the menu
#[derive(PartialEq, Debug, Format)]
pub enum Menu {
    Idle,       // the default state: the clock is displayed
    SetAlarm,   // the alarm is being set
    SystemInfo, // system info is being displayed
}

/// options for the system info
#[derive(PartialEq, Debug, Format)]
pub enum SystemInfo {
    Select,   // select to either display the system info or shutdown the system
    Info,     // display the system info
    Shutdown, // shutdown the system
}

impl StateManager {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            menu: Menu::Idle,
            system_info: SystemInfo::Select,
            alarm_time: (0, 0),
            alarm_enabled: false,
            power_state: PowerState::Battery { level: 0 },
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.menu = Menu::Idle;
        self.system_info = SystemInfo::Select;
        self.alarm_time = (0, 0);
        self.alarm_enabled = false;
        self.power_state = PowerState::Battery { level: 0 };
    }
}

#[derive(PartialEq, Debug, Format)]
pub enum PowerState {
    Battery {
        level: u8, // Battery level as a percentage
    },
    Power {
        usb_powered: bool, // true if the system is powered by USB
    },
}

/// Task to orchestrate the states of the system
/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// ToDo: in general we will be reacting to a number of event
/// - button presses, multiple things depending on the button and the state of the system
/// - alarm time reached
/// - plugging in usb power
/// and once we reached states we will need to trigger display updates, sound, etc.
#[embassy_executor::task]
pub async fn orchestrate(_spawner: Spawner, rtc_ref: &'static RefCell<Rtc<'static, RTC>>) {
    let state_manager = StateManager::new();
    let event_receiver = EVENT_CHANNEL.receiver();

    info!("Orchestrate task started");

    loop {
        info!("Orchestrate loop");

        // receive the events
        let event = event_receiver.receive().await;

        // react to the events
        match event {
            Events::BlueBtn(presses) => {
                info!("Blue button pressed, presses: {}", presses);
            }
            Events::GreenBtn(presses) => {
                info!("Green button pressed, presses: {}", presses);
            }
            Events::YellowBtn(presses) => {
                info!("Yellow button pressed, presses: {}", presses);
            }
            Events::Vbus(usb) => {
                info!("Vbus event, usb: {}", usb);
            }
            Events::Vsys(voltage) => {
                info!("Vsys event, voltage: {}", voltage);
            }
        }
        // match event {
        //     Ok(Events::BlueBtn(presses)) => {
        //         info!("Blue button pressed, presses: {}", presses);
        //     }
        //     Ok(Events::GreenBtn(presses)) => {
        //         info!("Green button pressed, presses: {}", presses);
        //     }
        //     Ok(Events::YellowBtn(presses)) => {
        //         info!("Yellow button pressed, presses: {}", presses);
        //     }
        //     Ok(Events::Vbus(usb)) => {
        //         info!("Vbus event, usb: {}", usb);
        //     }
        //     Ok(Events::Vsys(voltage)) => {
        //         info!("Vsys event, voltage: {}", voltage);
        //     }
        //     // more events
        //     _ => {}
        // }

        info!("StateManager: {:?}", state_manager);

        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "orhestrate loop: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }
    }
}
