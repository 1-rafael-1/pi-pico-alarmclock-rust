use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select_array;
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::Rtc;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

/// Channels for the events that we want to react to
/// we will need more channels for the other events, and we may need to use pipes instead of channels in some cases
/// see below in the orchestrate task for the ToDo
pub static GREEN_BTN_CHANNEL: Channel<CriticalSectionRawMutex, u8, 1> = Channel::new();
pub static BLUE_BTN_CHANNEL: Channel<CriticalSectionRawMutex, u8, 1> = Channel::new();
pub static YELLOW_BTN_CHANNEL: Channel<CriticalSectionRawMutex, u8, 1> = Channel::new();
pub static VBUS_CHANNEL: Channel<CriticalSectionRawMutex, u8, 1> = Channel::new();

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
    let mut state_manager = StateManager::new();

    let blue_btn_receiver = BLUE_BTN_CHANNEL.receiver();
    let green_btn_receiver = GREEN_BTN_CHANNEL.receiver();
    let yellow_btn_receiver = YELLOW_BTN_CHANNEL.receiver();
    let vbus_receiver = VBUS_CHANNEL.receiver();

    info!("Orchestrate task started");

    loop {
        info!("Orchestrate loop");

        // determine the state of the system by checking the button presses
        let blue_btn_future = blue_btn_receiver.receive();
        let green_btn_future = green_btn_receiver.receive();
        let yellow_btn_future = yellow_btn_receiver.receive();

        // determine the state of the system by checking the power state
        let vbus_future = vbus_receiver.receive();

        let futures = [
            blue_btn_future,
            green_btn_future,
            yellow_btn_future,
            vbus_future,
        ];

        match select_array(futures).await {
            (_, 0) => {
                info!("BLUE");
            }
            (_, 1) => {
                info!("GREEN");
                state_manager.state.toggle_alarm_active();
            }
            (_, 2) => {
                info!("YELLOW");
            }
            (_, 3) => {
                info!("VBUS");
            }
            _ => {
                info!("unreachable");
            }
        }

        info!("StateMansger: {:?}", state_manager);

        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "orhestrate loop: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }
    }
}
