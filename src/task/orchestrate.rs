//! # Orchestrate Tasks
//! Task to orchestrate the state transitions of the system.
use crate::task::state::*;
use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals::RTC;
use embassy_rp::rtc::DayOfWeek;
use embassy_rp::rtc::{DateTime, Rtc};
use embassy_time::{Duration, Timer};

/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// It receives events from the other tasks and reacts to them by changing the state of the system.
#[embassy_executor::task]
pub async fn orchestrate(_spawner: Spawner) {
    info!("Orchestrate task starting");
    // initialize the state manager and put it into the mutex
    {
        let state_manager = StateManager::new();
        *(STATE_MANAGER_MUTEX.lock().await) = Some(state_manager);
    }

    let event_receiver = EVENT_CHANNEL.receiver();
    let flash_sender = FLASH_CHANNEL.sender();

    loop {
        // receive the events, halting the task until an event is received
        let event = event_receiver.receive().await;

        '_mutex_guard: {
            // Lock the mutex to get a mutable reference to the state manager
            let mut state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
            // Get a mutable reference to the state manager. We can unwrap here because we know that the state manager is initialized
            let state_manager = state_manager_guard.as_mut().unwrap();

            // react to the events
            match event {
                Events::BlueBtn(_presses) => {
                    state_manager.handle_blue_button_press().await;
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::GreenBtn(_presses) => {
                    state_manager.handle_green_button_press().await;
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::YellowBtn(_presses) => {
                    state_manager.handle_yellow_button_press().await;
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::Vbus(usb) => {
                    info!("Vbus event, usb: {}", usb);
                    state_manager.power_state.set_usb_power(usb);
                    state_manager.power_state.set_battery_level();
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::Vsys(voltage) => {
                    info!("Vsys event, voltage: {}", voltage);
                    state_manager.power_state.set_vsys(voltage);
                    state_manager.power_state.set_battery_level();
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::AlarmSettingsReadFromFlash(alarm_settings) => {
                    info!("Alarm time read from flash: {:?}", alarm_settings);
                    state_manager.alarm_settings = alarm_settings;
                }
                Events::Scheduler((hour, minute, second)) => {
                    info!("Scheduler event");
                    if !state_manager.alarm_settings.get_enabled() {
                        NEOPIXEL_CHANNEL
                            .sender()
                            .send(Commands::NeopixelUpdate((hour, minute, second)))
                            .await;
                    }
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::RtcUpdated => {
                    info!("RTC updated event");
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::AlarmSettingsNeedUpdate => {
                    info!("Alarm settings must be updated event");
                    flash_sender
                        .send(Commands::AlarmSettingsWriteToFlash(
                            state_manager.alarm_settings.clone(),
                        ))
                        .await;
                }
                Events::Standby => {
                    info!("Standby event");
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                    TIMER_STOP_SIGNAL.signal(Commands::MinuteTimerStop);
                }
                Events::WakeUp => {
                    info!("Wake up event");
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                    TIMER_START_SIGNAL.signal(Commands::MinuteTimerStart);
                }
                Events::Alarm => {
                    info!("Alarm event");
                    state_manager.randomize_alarm_stop_buttom_sequence();
                    state_manager.set_alarm_mode();
                    // ToDo:
                    // 1. send the state to the sound task
                    // 2. send the state to the neopixel task
                    // 4. handle the alarm stop sequence
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                    NEOPIXEL_CHANNEL
                        .sender()
                        .send(Commands::NeopixelUpdate((0, 0, 0)))
                        .await;
                }
                Events::AlarmStop => {
                    info!("Alarm stop event");
                    state_manager.set_normal_mode();
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
            }
            // log the state of the system
            info!("{:?}", state_manager);
        }

        // ToDo: send the state to the sound task. This will be straightforward, as there is only one sound to play, the alarm sound.

        // ToDo: send the state to the neopixel task. This will need a little thinking, as the neopixel hs different effects to display
    }
}

/// This is the task that will handle scheduling timed events by sending events to the Event Channel when a given
/// time has passed. It will also handle the alarm event.
#[embassy_executor::task]
pub async fn scheduler(_spawner: Spawner, rtc_ref: &'static RefCell<Rtc<'static, RTC>>) {
    info!("scheduler task started");
    loop {
        // see if we must halt the task, then wait for the start signal
        if TIMER_STOP_SIGNAL.signaled() {
            info!("Minute timer task halted");
            TIMER_STOP_SIGNAL.reset();
            TIMER_START_SIGNAL.wait().await;
            info!("Minute timer task resumed");
        }

        let dt = match rtc_ref.borrow().now() {
            Ok(dt) => dt,
            Err(e) => {
                info!("RTC not running: {:?}", Debug2Format(&e));
                // return an empty DateTime
                DateTime {
                    year: 0,
                    month: 0,
                    day: 0,
                    day_of_week: DayOfWeek::Monday,
                    hour: 0,
                    minute: 0,
                    second: 0,
                }
            }
        };

        EVENT_CHANNEL
            .sender()
            .send(Events::Scheduler((dt.hour, dt.minute, dt.second)))
            .await;

        // get the state of the system out of the mutex and quickly drop the mutex
        let state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
        let state_manager = match state_manager_guard.clone() {
            Some(state_manager) => state_manager,
            None => {
                error!("State manager not initialized");
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };
        drop(state_manager_guard);

        // calculate the downtime
        let mut downtime: Duration;
        if state_manager.alarm_settings.get_enabled() {
            // wait for 1 minute, unless we are in proximity of the alarm time, in which case we wait for 10 seconds
            let alarm_time_minutes = state_manager.alarm_settings.get_hour() * 60
                + state_manager.alarm_settings.get_minute();
            let current_time_minutes = (dt.hour * 60) + dt.minute;
            if (alarm_time_minutes - current_time_minutes) <= 3 {
                downtime = Duration::from_secs(10);
            } else {
                downtime = Duration::from_secs(60);
            }
        } else {
            // if the alarm is not enabled, we will be using the neopixel analog clock effect, which will need to be updated often
            // so we will wait for 3.75 seconds (60s / 16leds -> 3.75s until we must update the leds)
            downtime = Duration::from_millis(3750);
        }

        // raise the alarm event
        if state_manager.operation_mode != OperationMode::Alarm
            && state_manager.alarm_settings.get_enabled()
            && state_manager.alarm_settings.get_hour() == dt.hour
            && state_manager.alarm_settings.get_minute() == dt.minute
        {
            EVENT_CHANNEL.sender().send(Events::Alarm).await;
            // wait for slightly more than a minute, to avoid the alarm being raised again when the user was really quick to stop it
            downtime = Duration::from_secs(61);
        }

        info!("Scheduler task sleeping for {:?}", downtime);
        Timer::after(downtime).await;
    }
}
