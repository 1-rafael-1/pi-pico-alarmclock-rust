use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::Rtc;
use embassy_time::{Duration, Timer};

#[derive(PartialEq, Debug, Format)]
pub struct StateManager {
    pub state: State,
    pub menu: Menu,
    pub system_info: SystemInfo,
    pub alarm_time: (u8, u8),
    pub alarm_enabled: bool,
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
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.menu = Menu::Idle;
        self.system_info = SystemInfo::Select;
        self.alarm_time = (0, 0);
        self.alarm_enabled = false;
    }
}

/// Task to orchestrate the states of the system
/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
#[embassy_executor::task]
pub async fn orchestrate(_spawner: Spawner, rtc_ref: &'static RefCell<Rtc<'static, RTC>>) {
    let mut state_manager = StateManager::new();

    loop {
        if let Ok(dt) = rtc_ref.borrow_mut().now() {
            info!(
                "orhestrate loop: {}-{:02}-{:02} {}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            );
        }

        info!("StateMansger: {:?}", state_manager);

        Timer::after(Duration::from_secs(10)).await;
    }
}
