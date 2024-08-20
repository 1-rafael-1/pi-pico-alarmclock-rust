//! # Orchestrate Tasks
//! Task to orchestrate the state transitions of the system.
use crate::task::state::*;
use crate::task::task_messages::*;
use crate::task::time_updater::RTC_MUTEX;
use defmt::*;
use embassy_futures::select::select;
use embassy_rp::rtc::DateTime;
use embassy_rp::rtc::DayOfWeek;
use embassy_time::{Duration, Timer};

/// This task is responsible for the state transitions of the system. It acts as the main task of the system.
/// It receives events from the other tasks and reacts to them by changing the state of the system.
#[embassy_executor::task]
pub async fn orchestrator() {
    info!("Orchestrate task starting");
    // initialize the state manager and put it into the mutex
    {
        let state_manager = StateManager::new();
        *(STATE_MANAGER_MUTEX.lock().await) = Some(state_manager);
    }

    // init the receiver for the event channel, this is the line we are listening on
    let event_receiver = EVENT_CHANNEL.receiver();

    // init the sender for the flash channel
    let flash_sender = FLASH_CHANNEL.sender();

    loop {
        // receive the events, halting the task until an event is received
        let event = event_receiver.receive().await;

        '_state_manager_mutex: {
            // Lock the mutex to get a mutable reference to the state manager
            let mut state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
            // Get a mutable reference to the state manager. We can unwrap here because we know that the state manager is initialized
            let state_manager = state_manager_guard.as_mut().unwrap();

            // react to the events
            match event {
                Events::BlueBtn => {
                    state_manager.handle_blue_button_press().await;
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::GreenBtn => {
                    state_manager.handle_green_button_press().await;
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::YellowBtn => {
                    state_manager.handle_yellow_button_press().await;
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                }
                Events::Vbus(usb) => {
                    info!("Vbus event, usb: {}", usb);
                    state_manager.power_state.set_usb_power(usb);
                    if !state_manager.power_state.get_usb_power() {
                        VSYS_WAKE_SIGNAL.signal(Commands::VsysWakeUp);
                    };
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
                    // update the light effects if the alarm is not enabled and the alarm state is None
                    if state_manager.alarm_state == AlarmState::None
                        && !state_manager.alarm_settings.get_enabled()
                    {
                        LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((hour, minute, second)));
                    }
                    // update the display
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
                    if state_manager.alarm_settings.get_enabled() {
                        // if the alarm is enabled, we must update the light effects once more
                        LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
                    } else {
                        // if the alarm is disabled, we must wake up the scheduler early
                        SCHEDULER_WAKE_SIGNAL.signal(Commands::SchedulerWakeUp);
                    }
                }
                Events::Standby => {
                    info!("Standby event");
                    SCHEDULER_STOP_SIGNAL.signal(Commands::SchedulerStop);
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                    LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
                    if SOUND_START_SIGNAL.signaled() {
                        SOUND_STOP_SIGNAL.signal(Commands::SoundUpdate);
                    }
                    TIME_UPDATER_SUSPEND_SIGNAL.signal(Commands::TimeUpdaterSuspend);
                }
                Events::WakeUp => {
                    info!("Wake up event");
                    SCHEDULER_START_SIGNAL.signal(Commands::SchedulerStart);
                    VSYS_WAKE_SIGNAL.signal(Commands::VsysWakeUp);
                    TIME_UPDATER_SUSPEND_SIGNAL.reset();
                    TIME_UPDATER_RESUME_SIGNAL.signal(Commands::TimeUpdaterResume);
                }
                Events::Alarm => {
                    info!("Alarm event");
                    state_manager.randomize_alarm_stop_buttom_sequence();
                    state_manager.set_alarm_mode();
                    DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                    LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
                    ALARM_EXPIRER_SIGNAL.signal(Commands::AlarmExpiry);
                }
                Events::AlarmStop => {
                    info!("Alarm stop event");
                    if state_manager.alarm_state.is_active() {
                        state_manager.set_normal_mode();
                        DISPLAY_SIGNAL.signal(Commands::DisplayUpdate);
                        LIGHTFX_STOP_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
                        LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
                        SOUND_STOP_SIGNAL.signal(Commands::SoundUpdate);
                    };
                }
                Events::SunriseEffectFinished => {
                    info!("Sunrise effect finished event");
                    state_manager.set_alarm_state(AlarmState::Noise);
                    SOUND_START_SIGNAL.signal(Commands::SoundUpdate);
                    LIGHTFX_SIGNAL.signal(Commands::LightFXUpdate((0, 0, 0)));
                }
            }
            drop(state_manager_guard);
        }
    }
}

/// This is the task that will handle scheduling timed events by sending events to the Event Channel when a given
/// time has passed. It will also handle the alarm event.
#[embassy_executor::task]
pub async fn scheduler() {
    info!("scheduler task started");
    'mainloop: loop {
        // see if we must halt the task, then wait for the start signal
        if SCHEDULER_STOP_SIGNAL.signaled() {
            SCHEDULER_STOP_SIGNAL.reset();
            SCHEDULER_START_SIGNAL.wait().await;
        };

        // get the current time
        let dt: DateTime;
        '_rtc_mutex: {
            let rtc_guard = RTC_MUTEX.lock().await;
            let rtc = match rtc_guard.as_ref() {
                Some(rtc) => rtc,
                None => {
                    error!("RTC not initialized");
                    drop(rtc_guard);
                    Timer::after(Duration::from_secs(3)).await;
                    continue 'mainloop;
                }
            };
            dt = match rtc.now() {
                Ok(dt) => dt,
                Err(e) => {
                    info!("RTC not running: {:?}", Debug2Format(&e));
                    // Return an empty DateTime
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
        };

        EVENT_CHANNEL
            .sender()
            .send(Events::Scheduler((dt.hour, dt.minute, dt.second)))
            .await;

        // get the state of the system out of the mutex and quickly drop the mutex
        let state_manager: StateManager;
        '_state_manager_mutex: {
            let state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
            state_manager = match state_manager_guard.clone() {
                Some(state_manager) => state_manager,
                None => {
                    error!("State manager not initialized");
                    drop(state_manager_guard);
                    Timer::after(Duration::from_secs(1)).await;
                    continue 'mainloop;
                }
            };
        }

        // calculate the downtime we need to wait until the next iteration
        let mut downtime: Duration;
        if state_manager.alarm_settings.get_enabled() {
            // wait for 1 minute, unless we are in proximity of the alarm time, in which case we wait for 10 seconds
            let alarm_time_minutes = state_manager.alarm_settings.get_hour() as u32 * 60
                + state_manager.alarm_settings.get_minute() as u32;
            let current_time_minutes = (dt.hour as u32 * 60) + dt.minute as u32;
            if alarm_time_minutes > current_time_minutes {
                // if the alarm time is in the future, we wait for 60 seconds, unless we are in the proximity of the alarm time
                if (alarm_time_minutes - current_time_minutes) <= 3 {
                    downtime = Duration::from_secs(10);
                } else {
                    downtime = Duration::from_secs(60);
                }
            } else {
                // if the alarm time is in the past, we wait for 60 seconds
                downtime = Duration::from_secs(60);
            }
        } else {
            // if the alarm is not enabled, we will be using the neopixel analog clock effect, which will need to be updated often
            // so we must wait for 3.75 seconds (60s / 16leds -> 3.75s until we must update the leds). To avoid visual glitches, we reduce that time by 10ms
            downtime = Duration::from_millis(3740);
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

        // we either wait for the downtime or until we are woken up early. Whatever comes first, starts the next iteration.
        let downtime_timer = Timer::after(downtime);
        select(downtime_timer, SCHEDULER_WAKE_SIGNAL.wait()).await;
    }
}

/// This task handles the expiration of the alarm after 5 minutes.
#[embassy_executor::task]
pub async fn alarm_expirer() {
    info!("Alarm expirer task started");
    '_mainloop: loop {
        // wait for the alarm expiry watcher signal
        ALARM_EXPIRER_SIGNAL.wait().await;
        // wait for 5 minutes
        Timer::after(Duration::from_secs(300)).await;
        // send the alarm stop event
        EVENT_CHANNEL.sender().send(Events::AlarmStop).await;
    }
}
