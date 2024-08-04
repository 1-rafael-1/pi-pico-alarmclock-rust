//! # Orchestrate Tasks
//! Task to orchestrate the state transitions of the system.
use crate::task::state::*;
use core::cell::RefCell;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals::RTC;
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
                    state_manager.set_alarm_mode();
                    // ToDo:
                    // 1. send the state to the sound task
                    // 2. send the state to the neopixel task
                    // 3. make the alarm stop sequence
                    // 4. handle the alarm stop sequence
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::AlarmStop => {
                    info!("Alarm stop event");
                    state_manager.set_normal_mode();
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
            }
        }

        // log the state of the system
        match _spawner.spawn(info(_spawner)) {
            Ok(_) => {}
            Err(_) => error!("info_task spawn failed"),
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

        let dt: DateTime = rtc_ref.borrow().now().unwrap();
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
        let downtime: Duration;
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
        if state_manager.alarm_settings.get_enabled()
            && state_manager.alarm_settings.get_hour() == dt.hour
            && state_manager.alarm_settings.get_minute() == dt.minute
        {
            EVENT_CHANNEL.sender().send(Events::Alarm).await;
        }

        Timer::after(downtime).await;
    }
}

/// Task to log the state of the system.
///
/// This task is responsible for logging the state of the system. It is triggered by the orchestrate task.
/// This is just a simple way to prove the Mutex is working as expected.
#[embassy_executor::task]
pub async fn info(_spawner: Spawner) {
    let mut state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
    match state_manager_guard.as_mut() {
        Some(state_manager) => {
            info!("{:?}", state_manager);
        }
        None => {
            info!("Info task started, but the state manager is not initialized yet");
        }
    }
}
